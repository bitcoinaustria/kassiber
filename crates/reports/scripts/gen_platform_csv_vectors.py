import json
from pathlib import Path
import sys
import tempfile
import sqlite3

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import _report_hooks
from kassiber.core import transaction_formats, export_range
from kassiber.db import open_db
from kassiber.errors import AppError

OUT = ROOT / 'crates/reports/tests/data/platform_csv.json'
BASE = "DELETE FROM journal_quarantines; DELETE FROM journal_entries WHERE id='swap';"
RANGE = dict(preset='custom', timezone='UTC', start='2024-01-01T00:00:00Z', end='2025-01-01T00:00:00Z', first_day='2024-01-01', last_day='2024-12-31', label='2024')


def resolved(payload):
    if payload is None:
        return None
    return export_range.resolve_export_range({'tax_country': 'generic'}, first_day=payload['first_day'], last_day=payload['last_day'], timezone_name=payload['timezone'])


def reference(conn, path, args):
    try:
        return dict(ok=transaction_formats.export_platform_csv(conn, args.get('workspace'), args.get('profile'), path, _report_hooks(), platform=args['platform'], wallet_ref=args.get('wallet'), wallet_refs=args.get('wallets'), export_range=resolved(args.get('export_range'))))
    except AppError as error:
        return dict(error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details, retryable=error.retryable))
    except (ValueError, TypeError, OverflowError, AttributeError, sqlite3.Error) as error:
        return dict(error=dict(code='internal_error', message=str(error), hint=None, details=None, retryable=False))


def changes():
    cases = [('base', {}, '')]
    for kind in ['income', 'mining', 'mining_reward', 'staking', 'airdrop', 'interest', 'lending_interest', 'LENDING-INTEREST', 'buy', 'sell', 'unknown']:
        entry = 'acquisition' if kind == 'buy' else 'disposal' if kind == 'sell' else 'income'
        direction = 'outbound' if kind == 'sell' else 'inbound'
        sql = f"INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,occurred_at,entry_type,asset,quantity,fiat_value,fiat_value_exact,proceeds,created_at) VALUES('new','ws','p','in','cold','2024-01-01T00:00:00Z','{entry}','BTC',123456789001,3,'3.123456789123456789',8,'fixed'); UPDATE transactions SET kind='{kind}',direction='{direction}',fiat_currency='EUR',fiat_value=30000,fiat_value_exact='30000.123456789123456789' WHERE id='in';"
        cases.append(('event-'+kind, {}, sql))
        if kind == 'buy':
            cases.append(('foreign-fiat', {}, sql+"UPDATE transactions SET fiat_currency='USD' WHERE id='in';"))
    for name, sql in [
        ('unbooked', 'DELETE FROM journal_entries;'),
        ('conversion-buy', "UPDATE journal_entries SET entry_type='acquisition' WHERE transaction_id='earn'; UPDATE journal_custody_economic_relations SET target_transaction_id='earn';"),
        ('no-relations', 'DELETE FROM journal_custody_economic_relations; DELETE FROM journal_custody_decisions;'),
        ('fee', "DELETE FROM journal_entries WHERE transaction_id='out' AND entry_type!='fee';"),
        ('fee-included', "UPDATE transactions SET amount_includes_fee=1 WHERE id='out';"),
        ('fee-only-included', "DELETE FROM journal_entries WHERE transaction_id='out' AND entry_type!='fee'; UPDATE transactions SET amount_includes_fee=1 WHERE id='out';"),
        ('zero-fee', 'UPDATE transactions SET fee=0;'),
        ('inbound-included', "UPDATE transactions SET amount_includes_fee=1 WHERE id='in';"),
        ('overflow-movement', "UPDATE transactions SET amount=9223372036854775807,fee=9223372036854775807 WHERE id='out';"),
        ('one-msat', 'UPDATE transactions SET amount=1,fee=1;'),
        ('large-msat', 'UPDATE transactions SET amount=9007199254740993;'),
        ('zero-msat', 'UPDATE transactions SET amount=0;'),
        ('negative-msat', 'UPDATE transactions SET amount=-12,fee=-2;'),
        ('non-taxable-conversion', "UPDATE transactions SET taxability_override=0 WHERE id='out';"),
        ('non-taxable-income', "UPDATE transactions SET taxability_override=0 WHERE id='earn';"),
        ('neutral', "UPDATE journal_entries SET at_category='neu_swap' WHERE transaction_id='out' AND entry_type='disposal';"),
        ('mixed-income', "UPDATE journal_entries SET entry_type='income' WHERE id='first';"),
        ('mixed-transfer', "UPDATE journal_entries SET entry_type='transfer_out' WHERE id='first';"),
        ('transfer', "UPDATE journal_entries SET entry_type='transfer_out' WHERE transaction_id='out';"),
        ('neutral-transfer', "UPDATE journal_entries SET entry_type='transfer_out',at_category='neu_swap' WHERE transaction_id='out';"),
        ('no-value', 'UPDATE transactions SET fiat_rate=NULL,fiat_rate_exact=NULL,fiat_value=NULL,fiat_value_exact=NULL; DELETE FROM journal_entries;'),
        ('zero-value', 'UPDATE journal_entries SET fiat_value=0,fiat_value_exact=\'0\',proceeds=0,proceeds_exact=\'0\';'),
        ('negative-zero', "UPDATE journal_entries SET fiat_value_exact='-0',proceeds_exact='-0';"),
        ('exact', "UPDATE journal_entries SET fiat_value_exact='1234567890.12345678901234567890123456789',proceeds_exact='0.12345678901234567890123456789';"),
        ('empty-exact', "UPDATE journal_entries SET fiat_value_exact='',proceeds_exact='';"),
        ('invalid-exact', "UPDATE transactions SET fiat_value_exact='bad' WHERE id='in';"),
        ('nonfinite', "UPDATE transactions SET fiat_rate_exact='NaN',fiat_value_exact='Infinity';"),
        ('issued-assets', "UPDATE transactions SET asset=printf('%064X',255) WHERE id='out';"),
        ('asset-alias', "UPDATE transactions SET asset='l-btc' WHERE id='out';"),
        ('namespaced', "UPDATE transactions SET asset='ext:token:usdt' WHERE id='out';"),
        ('unicode-asset', "UPDATE transactions SET asset='ext:token:ᲊ' WHERE id='out';"),
        ('unicode-bare', "UPDATE transactions SET asset='ᲊ' WHERE id='out';"),
        ('canonical-btc', "UPDATE transactions SET asset='bitcoin:btc' WHERE id='out';"),
        ('unicode', "UPDATE wallets SET label='日本語, € 😀' WHERE id='cold'; UPDATE transactions SET description='  first,\"quote\"' || char(13,10) || 'next line　',external_id='t,\"😀' || char(13,10);"),
        ('override', "UPDATE transactions SET kind_override='  MINING REWARD  ' WHERE id='earn';"),
        ('offset', "UPDATE transactions SET occurred_at='2024-01-01T01:02:03.999999+02:00' WHERE id='in';"),
        ('naive', "UPDATE transactions SET occurred_at='2024-01-01T01:02:03.123456' WHERE id='in';"),
        ('week', "UPDATE transactions SET occurred_at='2024-W01-1T01:02:03Z' WHERE id='in';"),
        ('short-year', "UPDATE transactions SET occurred_at='0001-01-01T01:02:03Z' WHERE id='in';"),
        ('invalid-date', "UPDATE transactions SET occurred_at='bad' WHERE id='in';"),
        ('space-date', "UPDATE transactions SET occurred_at=' 2024-01-01T00:00:00Z' WHERE id='in';"),
        ('unknown-date', "UPDATE transactions SET occurred_at='1970-01-01T00:00:00Z' WHERE id='in';"),
        ('empty-date', "UPDATE transactions SET occurred_at='' WHERE id='in';"),
        ('stale', 'UPDATE profiles SET last_processed_input_version=-1;'),
        ('freshness', "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,blocking_reports,updated_at) VALUES('p','a','wallet','Cold','failed',1,'fixed');"),
        ('quarantined', "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('in','ws','p','missing_price','fixed');"),
        ('pair-ref', "UPDATE journal_custody_economic_relations SET relation_id='abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789',target_transaction_id='in';"),
        ('empty', 'DELETE FROM journal_entries; DELETE FROM transactions; UPDATE profiles SET last_processed_tx_count=0;'),
    ]:
        cases.append((name, {}, sql))
    for args in [dict(wallets=['Hot']), dict(wallets=['Cold', 'Hot', 'Cold']), dict(wallets=[]),dict(wallets=['missing']),dict(export_range=RANGE),dict(wallets=['Hot'],export_range=RANGE),dict(workspace='missing'),dict(profile='other')]:
        cases.append(('selection-'+str(len(cases)),args,''))
    for name, args, sql in list(cases):
        if name in ['empty-date','unknown-date','invalid-date','quarantined','offset']:
            cases.append((name+'-range',dict(args,export_range=RANGE),sql))
    for wallet in ['Hot','Cold']:
        cases.append(('quarantine-scope-'+wallet,dict(wallets=[wallet],export_range=RANGE),"INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('no-tax','ws','p','missing_price','fixed');"))
    cases.append(('price-outside-wallet',dict(wallets=['Hot']),"UPDATE transactions SET fiat_value_exact='bad' WHERE id='in';"))
    cases.append(('quarantine-undated',dict(export_range=RANGE),"UPDATE transactions SET occurred_at='' WHERE id='in'; INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('in','ws','p','missing_price','fixed');"))
    return cases


def build():
    result=[]
    for platform in ['cointracking','blockpit']:
        for name,args,sql in changes():
            with tempfile.TemporaryDirectory(dir=ROOT/'target/r5-platform/scratch') as raw:
                root=Path(raw)
                conn=open_db(root/'data')
                conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE+sql)
                args=dict(platform=platform,**args)
                target=root/'platform.csv'
                expected=reference(conn,target,args)
                if 'ok' in expected:
                    del expected['ok']['file']
                    content=target.read_bytes().decode('utf-8')
                else:
                    content=None
                result.append(dict(name=platform+'-'+name,sql=BASE+sql,args_json=json.dumps(args),expected=expected,content=content))
                conn.close()
    return result


if __name__ == '__main__':
    OUT.parent.mkdir(parents=True,exist_ok=True)
    (ROOT/'target/r5-platform/scratch').mkdir(parents=True,exist_ok=True)
    data=build()
    content='[\n'+',\n'.join(json.dumps(c,ensure_ascii=True,separators=(',',':')) for c in data)+'\n]\n'
    if '--check' in sys.argv:
        assert OUT.read_text()==content, 'platform CSV vectors drifted'
    else:
        OUT.write_text(content)
    print(f'{len(data)} platform CSV cases')
