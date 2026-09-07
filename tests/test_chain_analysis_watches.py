"""Real encrypted book, production graph and durable watch transaction tests."""
import json
import sqlite3
from types import SimpleNamespace
from unittest.mock import patch

import pytest
from sqlcipher3 import dbapi2 as cipher

from kassiber.core import chain_analysis_watches as watches
from kassiber.core.chain_analysis import build_index
from kassiber.core.chain_analysis_api import dispatch
from kassiber.core.chain_analysis_ai import project_ai_result
from kassiber.db import SCHEMA, ensure_database_instance_id, set_setting
from kassiber.errors import AppError
from kassiber.daemon_chain_analysis_watches import deliver_pending

PROFILE = 'profile'
TX = '1' * 64
OTHER = '2' * 64
DOMAIN = {'domain_id': 'domain', 'environment_id': 'environment', 'revision': 1, 'chain': 'bitcoin', 'network': 'main', 'chain_instance_id': None}


@pytest.fixture
def book(tmp_path):
    conn = cipher.connect(str(tmp_path / 'book.db'))
    conn.row_factory = cipher.Row
    conn.execute("PRAGMA key='test-local-watches'")
    conn.executescript(SCHEMA)
    ensure_database_instance_id(conn)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','2026-01-01')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book','2026-01-01')")
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','profile','Private wallet','address','{}','2026-01-01')")
    set_setting(conn,'context_workspace','ws'); set_setting(conn,'context_profile',PROFILE)
    put_tx(conn, TX, confirmations=0)
    conn.commit()
    # Independent domain/index PRs provide these adapters. Everything after
    # their stable token/domain boundary uses the real production graph.
    with patch.object(watches,'domain_for',return_value=DOMAIN), patch.object(watches,'revision_for',side_effect=lambda c,p: {'revision': 1, 'snapshot_id': build_index(c,p).snapshot_id, 'next_expiry': None, 'schema_version': 1}):
        yield conn
    conn.close()


def put_tx(conn, txid, *, confirmations=0, spend=None, label=None):
    raw = {'txid':txid,'confirmations':confirmations,'vin':[{'txid':spend,'vout':0,'prevout':{'value':100}}] if spend else [{'coinbase':'0101'}], 'vout':[{'value':90 if spend else 100,'scriptpubkey':'0014'+'11'*20}]}
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,occurred_at,direction,asset,amount,fee,created_at,raw_json) VALUES(?,'ws','profile','wallet',?,?,'2026-01-01','inbound','BTC',100000,0,'2026-01-01',?) ON CONFLICT(id) DO UPDATE SET raw_json=excluded.raw_json", (txid,txid,txid,json.dumps(raw)))


def definition(rule='output_spent'):
    return {'rule':rule,'query':{'mode':'trace','subject':f'{TX}:0' if rule=='output_spent' else TX,'chain':'bitcoin','network':'main'}}


def create(conn, args=None):
    plan=dispatch(conn,'ui.chain_analysis.watches.preview',args or definition())
    return dispatch(conn,'ui.chain_analysis.watches.create',{'plan':plan})


def tick(conn):
    return dispatch(conn,'ui.chain_analysis.watches.evaluate',{})


def inbox(conn):
    return dispatch(conn,'ui.chain_analysis.watches.inbox',{})


def test_encryption_requires_key_not_driver_capability():
    for module in (sqlite3,cipher):
        c=module.connect(':memory:')
        c.execute('create table sample(x)')
        with pytest.raises(AppError,match='Encrypt'):
            watches.require_encrypted(c)
        c.close()
    c=cipher.connect(':memory:');c.execute("pragma key='test'");c.execute('create table sample(x)')
    watches.require_encrypted(c);c.close()


def test_baseline_then_spend_is_durable_and_deduplicated(book):
    watch=create(book)
    assert inbox(book)['items']==[]
    put_tx(book,OTHER,spend=TX);book.commit()
    with patch('socket.create_connection',side_effect=AssertionError('network')):
        assert tick(book)['event_count']==1
        assert tick(book)['event_count']==0
    events=inbox(book)
    assert events['items'][0]['code']=='spend_observed'
    assert events['items'][0]['watch_id']==watch['id']
    dispatch(book,'ui.chain_analysis.watches.acknowledge',{'id':events['items'][0]['id']})
    assert inbox(book)['unread_count']==0


def test_pause_resume_catches_up_and_stale_config_is_rejected(book):
    watch=create(book)
    paused=dispatch(book,'ui.chain_analysis.watches.configure',{'id':watch['id'],'expected_revision':1,'enabled':False})
    put_tx(book,OTHER,spend=TX);book.commit();assert tick(book)['event_count']==0
    with pytest.raises(AppError):
        dispatch(book,'ui.chain_analysis.watches.configure',{'id':watch['id'],'expected_revision':1,'enabled':True})
    dispatch(book,'ui.chain_analysis.watches.configure',{'id':watch['id'],'expected_revision':paused['revision'],'enabled':True})
    assert tick(book)['event_count']==1


def test_stale_preview_does_not_create(book):
    plan=dispatch(book,'ui.chain_analysis.watches.preview',definition())
    put_tx(book,OTHER,spend=TX);book.commit()
    with pytest.raises(AppError):dispatch(book,'ui.chain_analysis.watches.create',{'plan':plan})
    assert dispatch(book,'ui.chain_analysis.watches.list',{})['items']==[]


def test_confirmation_threshold_reversal_and_no_alert_per_block(book):
    create(book,{**definition('confirmations'),'threshold':3})
    put_tx(book,TX,confirmations=2);book.commit();assert tick(book)['event_count']==0
    put_tx(book,TX,confirmations=3);book.commit();assert tick(book)['event_count']==1
    put_tx(book,TX,confirmations=4);book.commit();assert tick(book)['event_count']==0
    put_tx(book,TX,confirmations=0);book.commit();assert tick(book)['event_count']==1
    assert [row['code'] for row in inbox(book)['items']]==['threshold_reversed','threshold_reached']


def test_disappearing_spend_means_coverage_lost_never_unspent(book):
    put_tx(book,OTHER,spend=TX);book.commit();create(book)
    book.execute('DELETE FROM transactions WHERE id=?',(OTHER,));book.commit()
    assert tick(book)['event_count']==1
    assert inbox(book)['items'][0]['code']=='coverage_lost'


def test_atomic_failure_rolls_back_inbox_and_baseline(book):
    watch=create(book)
    put_tx(book,OTHER,spend=TX);book.commit()
    book.execute("CREATE TRIGGER reject_watch_update BEFORE UPDATE ON chain_analysis_watches BEGIN SELECT RAISE(ABORT,'failed checkpoint'); END")
    with pytest.raises(Exception,match='failed checkpoint'):tick(book)
    assert inbox(book)['items']==[]
    assert watches.get_watch(book,PROFILE,watch['id'])['baseline']==watch['baseline']
    book.execute('DROP TRIGGER reject_watch_update');book.commit();assert tick(book)['event_count']==1


def test_domain_change_and_cancel_fail_closed(book):
    create(book)
    with patch.object(watches,'domain_for',return_value={**DOMAIN,'revision':2}):
        tick(book)
        assert dispatch(book,'ui.chain_analysis.watches.list',{})['items'][0]['baseline']['reason']=='domain_changed'
    with pytest.raises(AppError):watches.evaluate_due(book,PROFILE,cancelled=lambda:True)


def test_generic_delivery_and_ai_projection_never_contain_subjects(book):
    create(book);put_tx(book,OTHER,spend=TX);book.commit();tick(book)
    calls=[];out=SimpleNamespace(write=calls.append)
    deliver_pending(book,out);deliver_pending(book,out)
    assert len(calls)==1 and calls[0]['event'] is True
    assert TX not in json.dumps(calls) and OTHER not in json.dumps(calls)
    assert 'request_id' not in calls[0]
    projected=project_ai_result(book,PROFILE,inbox(book))
    assert TX not in json.dumps(projected) and OTHER not in json.dumps(projected)


def test_partial_semantics_never_says_cleared():
    before={'status':'observed','values':['finding']}
    after={'status':'partial','values':[]}
    assert watches.transition(before,after,'findings_changed')=='coverage_lost'


def test_ai_compact_create_is_stale_bound_and_mutating(book):
    from kassiber.ai.tools import get_tool
    preview=dispatch(book,'ui.chain_analysis.watches.preview',definition())
    created=dispatch(book,'ui.chain_analysis.watches.create',{'definition':definition(),'expected_plan_id':preview['plan_id']})
    assert created['enabled']
    assert get_tool('ui.chain_analysis.watches.create').kind_class=='mutating'
    assert not get_tool('ui.chain_analysis.watches.create').egresses


def test_retracted_spender_is_not_a_new_spend(book):
    create(book)
    put_tx(book,OTHER,spend=TX,confirmations=-1);book.commit()
    assert tick(book)['event_count']==0


def test_attribution_change_uses_canonical_observer_projection(book):
    from kassiber.core.chain_analysis_cases import upsert_label
    owner=create(book,definition('attribution_changed'))
    public_definition=definition('attribution_changed');public_definition['query']['observer']='public'
    public=create(book,public_definition)
    upsert_label(book,PROFILE,{'subject':TX,'chain':'bitcoin','network':'main','label':'Private merchant','category':'merchant','source':'Private contract','confidence':'user_confirmed'})
    book.commit();tick(book)
    assert inbox(book)['items'][0]['watch_id']==owner['id']
    assert not any(item['watch_id']==public['id'] for item in inbox(book)['items'])


def test_database_reopen_preserves_baseline_and_delivery_is_retryable(book):
    create(book);put_tx(book,OTHER,spend=TX);book.commit();tick(book)
    path=book.execute('pragma database_list').fetchone()[2]
    other=cipher.connect(path);other.execute("pragma key='test-local-watches'");other.row_factory=cipher.Row
    try:
        assert tick(other)['event_count']==0
        with pytest.raises(OSError):deliver_pending(other,SimpleNamespace(write=lambda _: (_ for _ in ()).throw(OSError())))
        assert other.execute('SELECT COUNT(*) FROM chain_analysis_watch_inbox WHERE delivered_at IS NULL').fetchone()[0]==1
        output=[];deliver_pending(other,SimpleNamespace(write=output.append));assert len(output)==1
    finally:other.close()


def test_inbox_page_does_not_skip_equal_timestamps(book):
    create(book,{**definition('confirmations'),'threshold':1})
    for count in [1,0,1,0,1]:
        put_tx(book,TX,confirmations=count);book.commit();tick(book)
    book.execute("UPDATE chain_analysis_watch_inbox SET created_at='same'");book.commit()
    first=dispatch(book,'ui.chain_analysis.watches.inbox',{'limit':2})
    second=dispatch(book,'ui.chain_analysis.watches.inbox',{'limit':2,'before':first['next_cursor']})
    third=dispatch(book,'ui.chain_analysis.watches.inbox',{'limit':2,'before':second['next_cursor']})
    assert len({item['id'] for page in [first,second,third] for item in page['items']})==5


def test_delete_watch_leaves_raw_evidence(book):
    watch=create(book);put_tx(book,OTHER,spend=TX);book.commit();tick(book)
    dispatch(book,'ui.chain_analysis.watches.delete',{'id':watch['id'],'expected_revision':1})
    assert inbox(book)['items']==[]
    assert book.execute('SELECT COUNT(*) FROM transactions').fetchone()[0]==2


def test_worker_owns_connection_and_stops_on_lock(book):
    import threading
    from kassiber import daemon_chain_analysis_watches as worker
    path=book.execute('pragma database_list').fetchone()[2]
    entered=threading.Event();seen=[]
    def open_worker(root,**kwargs):
        seen.append(kwargs)
        conn=cipher.connect(path);conn.execute("pragma key='test-local-watches'");conn.row_factory=cipher.Row
        return conn
    ctx=SimpleNamespace(conn=book,data_root='fixture',db_passphrase='test-local-watches',out=SimpleNamespace(write=lambda _:None))
    with patch.object(worker,'open_db',side_effect=open_worker),patch.object(worker,'worker_tick',side_effect=lambda conn,**kwargs:entered.set()):
        worker.start_worker(ctx)
        assert entered.wait(2)
        assert seen[0]['expected_database_identity']
        assert seen[0]['require_existing_schema'] is True
        assert worker.stop_worker(ctx)
        assert ctx.watch_worker is None
        ctx.conn=None
        worker.start_worker(ctx)
        assert len(seen)==1


def test_cancel_after_inbox_insert_rolls_back_entire_pass(book):
    create(book);put_tx(book,OTHER,spend=TX);book.commit()
    calls=0
    def cancelled():
        nonlocal calls
        calls+=1
        return calls>1
    with pytest.raises(AppError):watches.evaluate_due(book,PROFILE,cancelled=cancelled)
    assert inbox(book)['items']==[]
    assert tick(book)['event_count']==1


def test_cli_uses_shared_local_watch_surface(book):
    import io
    from contextlib import redirect_stdout
    from kassiber.cli.main import build_parser
    from kassiber.cli.chain_analysis import dispatch as cli_dispatch
    args=build_parser().parse_args(['chain-analysis','watches','preview','--document',json.dumps(definition())])
    result=cli_dispatch(book,args)
    assert result['definition']['rule']=='output_spent'
    assert result['egress']=='none'


@pytest.mark.parametrize('saved_case',[False,True])
def test_canonical_preview_definition_is_directly_copyable_into_create(book,saved_case):
    args=definition()
    if saved_case:
        result=dispatch(book,'ui.chain_analysis.query',args['query'])
        saved=dispatch(book,'ui.chain_analysis.cases.save',{'title':'Source output','query':result['query'],'expected_snapshot_id':result['snapshot_id']})
        args={'rule':'output_spent','case_id':saved['id']}
    preview=dispatch(book,'ui.chain_analysis.watches.preview',args)
    created=dispatch(book,'ui.chain_analysis.watches.create',{'definition':preview['definition'],'expected_plan_id':preview['plan_id']})
    assert created['definition']==preview['definition']


@pytest.mark.parametrize('tamper',['version','boolean_version','query','extra'])
def test_copied_definition_requires_exact_canonical_shape(book,tamper):
    result=dispatch(book,'ui.chain_analysis.query',definition()['query'])
    saved=dispatch(book,'ui.chain_analysis.cases.save',{'title':'Case','query':result['query'],'expected_snapshot_id':result['snapshot_id']})
    preview=dispatch(book,'ui.chain_analysis.watches.preview',{'rule':'output_spent','case_id':saved['id']})
    supplied=json.loads(json.dumps(preview['definition']))
    if tamper=='version':supplied['rule_version']=2
    elif tamper=='boolean_version':supplied['rule_version']=True
    elif tamper=='query':supplied['query']['observer']='public'
    else:supplied['ignored']='not allowed'
    with pytest.raises(AppError):dispatch(book,'ui.chain_analysis.watches.create',{'definition':supplied,'expected_plan_id':preview['plan_id']})
    assert dispatch(book,'ui.chain_analysis.watches.list',{})['items']==[]
