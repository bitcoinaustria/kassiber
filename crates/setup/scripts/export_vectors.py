#!/usr/bin/env python3
"""Freeze synthetic Python setup responses and full affected-table state.

Identity and clock values are injected before execution; no result is masked.
Use --check to detect reference drift. This never opens a real book.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import sys
import tempfile
import uuid
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from importlib import import_module
cli = import_module("kassiber.cli.main")
from kassiber import daemon
from kassiber.db import open_db
from kassiber.errors import AppError

DESTINATION = Path(__file__).resolve().parents[1] / 'fixtures/python_setup.json'


def generate():
    parser = cli.build_parser()
    sequence = 0
    records = []
    def identity():
        nonlocal sequence
        sequence += 1
        return uuid.UUID(f'00000000-0000-4000-8000-{sequence:012x}')
    with tempfile.TemporaryDirectory(prefix='kassiber-setup-vectors-') as root:
        conn = open_db(Path(root) / 'data')
        def state():
            return {table: [dict(r) for r in conn.execute(f'SELECT * FROM {table} ORDER BY id')]
                    for table in ('workspaces', 'profiles', 'accounts', 'saved_views')} | {
                        'settings': [dict(r) for r in conn.execute("SELECT * FROM settings WHERE key LIKE 'context_%' ORDER BY key")]
                    }
        def record(argv=None, *, kind=None, args=None):
            captured = []
            if argv is not None:
                ns = parser.parse_args(['--machine', *argv])
                args = vars(ns)
                kind = '.'.join([ns.command, getattr(ns, ns.command+'_command')])
            try:
                if argv is not None:
                    with mock.patch.object(cli, 'emit', lambda _, payload, **kwargs: captured.append(payload)), mock.patch('kassiber.cli.handlers.emit', lambda _, payload, **kwargs: captured.append(payload)):
                        cli.dispatch(conn, ns)
                    value = dict(captured[0]) if hasattr(captured[0], 'keys') else captured[0]
                elif kind.startswith('ui.saved_views.'):
                    value = daemon._ui_swap_matching_payload_from_conn(conn, kind, args)
                else:
                    handler = {'create': daemon._create_profile_payload, 'rename': daemon._rename_profile_payload, 'update': daemon._update_profile_payload, 'switch': daemon._switch_profile_payload}[kind.rsplit('.',1)[1]]
                    value = handler(conn, args)
                outcome = {'ok': value}
            except AppError as exc:
                conn.rollback()
                outcome = {'error': {'code': exc.code, 'message': str(exc), 'hint': exc.hint, 'details': exc.details, 'retryable': exc.retryable}}
            except Exception as exc:
                conn.rollback()
                outcome = {'error': {'code':'internal_error','message':str(exc),'hint':None,'details':None,'retryable':False}}
            records.append({'command':kind, 'args':args, **outcome, 'state':state(), 'identity_calls':sequence})
            return outcome.get('ok')
        with mock.patch('uuid.uuid4', identity), mock.patch('kassiber.core.accounts.now_iso', return_value='2026-01-01T00:00:00Z'), mock.patch('kassiber.core.saved_views.now_iso', return_value='2026-01-01T00:00:00Z'):
            for argv in (['context','show'],['workspaces','list'],['profiles','list'],['workspaces','create','Main'],['workspaces','create','Main'],['workspaces','create','MAIN'],['profiles','list','--workspace','main'],['context','set','--workspace','Main'],['profiles','create','Book','--fiat-currency',' chf ','--tax-long-term-days','730'],['profiles','list'],['profiles','get'],['accounts','list'],['accounts','create','--code',' Savings Bucket ','--label','Savings','--type','ASSET','--asset','l-btc'],['accounts','create','--code','savings-bucket','--label','Dup','--type','asset'],['accounts','create','--code','bad','--label','Bad','--type',' asset '],['accounts','create','--code',' ','--label','Bad','--type','asset'],['profiles','set'],['profiles','set','--label','Renamed'],['profiles','set','--tax-country','at','--gains-algorithm','FIFO'],['profiles','set','--require-coarse-review','--no-bitcoin-rail-carrying-value'],['profiles','set','--tax-country','generic','--gains-algorithm','MOVING_AVERAGE_AT'],['profiles','set','--tax-country','generic','--gains-algorithm','HIFO','--tax-long-term-days','0'],['profiles','set','--tax-long-term-days','-1'],['profiles','set','--tax-country','invalid'],['profiles','set','--cost-basis-pool-scope','wallet'],['profiles','create','Invalid','--tax-long-term-days','-1'],['profiles','create','Renamed'],['profiles','create','Bad','--tax-country','at','--tax-long-term-days','9999999999999999999999999999'],['context','current'],['context','set'],['context','set','--workspace','MAIN','--profile','absent'],['context','show'],['views','create','--surface','swap','--name','One','--filter','{"z": 1, "a": ["é", null, true]}'],['views','list'],['views','create','--surface','swap','--name','One'],['views','create','--surface','other','--name','One'],['views','list','--surface','swap'],['views','create','--surface','swap','--name',' ','--filter','{}'],['views','create','--surface','swap','--name','Bad','--filter','['],['views','create','--surface','swap','--name','Pairs','--filter','[["key",1]]'],['views','create','--surface','swap','--name','Int key','--filter','[[1,2]]'],['views','create','--surface','swap','--name','Equal keys','--filter','[[1,2],[true,3],[1.0,4]]'],['views','create','--surface','swap','--name','No sequence','--filter','[1]'],['views','create','--surface','swap','--name','Wrong length','--filter','[[1,2,3]]'],['views','create','--surface','swap','--name','Scalar','--filter','true']):
                record(argv)
            record(kind='ui.profiles.create',args={'workspace_id':None,'label':'Bad'})
            current=record(['profiles','get'])
            wid=current['workspace_id']; pid=current['id']
            for country,algo in [('at','MOVING_AVERAGE_AT'),('generic','HIFO'),('at','FIFO'),('generic','FIFO')]:
                record(kind='ui.profiles.create',args={'workspace_id':wid,'label':country+algo,'tax_country':country,'gains_algorithm':algo})
            record(kind='ui.profiles.create',args={'workspace_id':wid,'label':'Copy','source_profile_id':pid,'tax_country':'at','gains_algorithm':'FIFO'})
            for args in ({'profile_id':pid,'tax_country':'at','gains_algorithm':'LIFO'},{'profile_id':pid,'tax_country':' '},{'profile_id':pid},{'profile_id':pid,'tax_country':'bad','gains_algorithm':'bad'}):
                record(kind='ui.profiles.update',args=args)
            record(kind='ui.profiles.switch',args={'profile_id':pid})
            record(kind='ui.profiles.rename',args={'profile_id':pid,'label':'Desktop renamed'})
            record(kind='ui.profiles.switch',args={'profile_id':'absent'})
            record(kind='ui.saved_views.create',args={'surface':'desktop','name':'Desktop view','filter':{'memo':'é'}})
            record(kind='ui.saved_views.list',args={})
            record(kind='ui.saved_views.create',args={'surface':'desktop','name':'Bad','filter':[1]})
            record(kind='ui.saved_views.delete',args={})
            vs=record(['views','list'])
            record(['views','delete','--view-id',vs[0]['id']])
            record(kind='ui.saved_views.delete',args={'view_id':vs[0]['id']})
            record(['profiles','create','Other'])
            record(['views','delete','--view-id',vs[1]['id']])
            record(['workspaces','create','Other'])
            record(['context','show'])
            record(['context','set','--workspace','Main','--profile','Desktop renamed'])
        conn.close()
    return {'reference':'82f6b77be04cbd1507a4145d54606efde940aa88', 'clock':'2026-01-01T00:00:00Z', 'records':records}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    text=json.dumps(generate(),indent=2,ensure_ascii=True)+'\n'
    if args.check:
        if DESTINATION.read_text()!=text:
            print('Python setup vectors drifted',file=sys.stderr)
            return 1
    else:
        DESTINATION.write_text(text)
    return 0
if __name__=='__main__':
    raise SystemExit(main())
