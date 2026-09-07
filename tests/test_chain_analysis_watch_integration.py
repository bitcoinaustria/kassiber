"""Encrypted end-to-end watches with real book binding and durable projection.

Only the node/provider transport is synthetic. No graph, domain, revision,
consent, persistence, projection or watch evaluator is replaced.
"""
import json
import queue
from types import SimpleNamespace
from unittest.mock import patch

import pytest

from kassiber import daemon
from kassiber.core import chain_analysis_acquisition as acquisition
from kassiber.core.book_network import apply_book_network, plan_book_network
from kassiber.core.chain_analysis.projection import index_revision
from kassiber.core.chain_analysis_api import dispatch
from kassiber.db import open_db, set_setting
from kassiber.daemon_chain_analysis_watches import deliver_pending, worker_tick
from tests.test_chain_analysis_acquisition import FakeHTTP, raw, tid
from tests.test_chain_analysis_consent import ConsentOutput

SECRET = 'local-watch-integration-passphrase'


@pytest.fixture
def encrypted_book(tmp_path):
    conn = open_db(str(tmp_path), passphrase=SECRET)
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Main','2026-01-01')")
    conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('profile','ws','Book','2026-01-01')")
    conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','profile','Local wallet','address',?, '2026-01-01')", (json.dumps({'chain':'bitcoin','network':'main'}),))
    set_setting(conn, 'context_workspace', 'ws'); set_setting(conn, 'context_profile', 'profile')
    payload=raw(1)
    conn.execute("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,occurred_at,direction,asset,amount,fee,created_at,raw_json) VALUES('seed','ws','profile','wallet','seed',?,'2026-01-01','inbound','BTC',100000,0,'2026-01-01',?)", (tid(1),json.dumps(payload)))
    conn.execute("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES('node','esplora','bitcoin','main','http://127.0.0.1:18443/api','2026-01-01','2026-01-01')")
    args={'environment':'main','declared_wallet_ids':['wallet']}
    plan=plan_book_network(conn,'profile',args)
    assert plan['can_apply'],plan['blockers']
    apply_book_network(conn,'profile',{**args,'plan_id':plan['plan_id']})
    conn.commit()
    yield conn,tmp_path
    conn.close()


def watch_definition():
    return {'rule':'output_spent','query':{'mode':'trace','subject':f'{tid(1)}:0','chain':'bitcoin','network':'main','observer':'public'}}


def create_watch(conn):
    preview=dispatch(conn,'ui.chain_analysis.watches.preview',watch_definition())
    return dispatch(conn,'ui.chain_analysis.watches.create',{'plan':preview})


def read_inbox(conn):
    return dispatch(conn,'ui.chain_analysis.watches.inbox',{})


def acquire_successor(conn):
    plan=acquisition.plan_acquisition(conn,'profile',{'backend':'node','subject':tid(2),'chain':'bitcoin','network':'main','direction':'backward','depth':1,'max_transactions':1})
    source=FakeHTTP({'/block-height/0':acquisition.GENESIS[('bitcoin','main')].encode(),f'/tx/{tid(2)}':raw(2,parents=(1,))})
    with patch.object(acquisition.transport,'urlopen_with_proxy',side_effect=source):
        acquisition.apply_acquisition(conn,'profile',{'plan':plan})
    conn.commit()
    return source


def test_acquisition_revision_inbox_and_encrypted_reopen_are_exactly_once(encrypted_book):
    conn,root=encrypted_book
    watch=create_watch(conn)
    before=index_revision(conn,'profile');conn.commit()
    assert read_inbox(conn)['unread_count']==0
    with patch('socket.getaddrinfo',side_effect=AssertionError('DNS')),patch('socket.socket.connect',side_effect=AssertionError('socket')):
        source=acquire_successor(conn)
        after=index_revision(conn,'profile');conn.commit()
        assert after['revision']>before['revision']
        assert after['snapshot_id']!=before['snapshot_id']
        worker_tick(conn)
        first=read_inbox(conn)
        assert [item['code'] for item in first['items']]==['spend_observed']
        assert first['items'][0]['watch_id']==watch['id']
        worker_tick(conn)
        assert read_inbox(conn)==first
    assert len(source.calls)==2
    # Release every keyed connection, as lock does, before reopening the book.
    conn.close()
    reopened=open_db(str(root),passphrase=SECRET,require_existing_schema=True)
    try:
        worker_tick(reopened)
        assert read_inbox(reopened)==first
        messages=[]
        deliver_pending(reopened,SimpleNamespace(write=messages.append))
        deliver_pending(reopened,SimpleNamespace(write=messages.append))
        assert len(messages)==1
        assert tid(1) not in json.dumps(messages) and tid(2) not in json.dumps(messages)
        assert index_revision(reopened,'profile')==after
    finally:reopened.close()


@pytest.mark.parametrize('decision',['allow_once','deny'])
@pytest.mark.parametrize('on_device',[True,False])
def test_real_agent_loop_requires_mutation_consent_for_watch(encrypted_book,decision,on_device):
    conn,root=encrypted_book
    preview=dispatch(conn,'ui.chain_analysis.watches.preview',watch_definition())
    conn.commit()
    runtime=daemon.AiToolRuntime(str(root),{},queue.Queue(),{'scope_workspace_id':'ws','scope_profile_id':'profile','provider_kind':'local' if on_device else 'remote','provider_on_device':on_device})
    chats=daemon.ActiveAiChats();_,active=chats.register('watch-consent')
    entry=daemon.get_tool('ui.chain_analysis.watches.create')
    validated=daemon._ai_chat_args({'model':'synthetic-local','tools_enabled':True,'messages':[{'role':'user','content':'Use chain analysis to create a local evidence watch for this output after asking permission.'}]})
    prompts=[]
    def approve(data):
        assert conn.execute('SELECT count(*) FROM chain_analysis_watches').fetchone()[0]==0
        prompts.append(data)
        assert active.consent.record(data['call_id'],decision)
    output=ConsentOutput(approve)
    calls=0
    def provider(*args):
        nonlocal calls
        if calls:
            return daemon.AiToolTurnResult([],'Done.','','stop',[])
        calls+=1
        assert entry.provider_name in {tool['name'] for tool in args[4]}
        call={'id':'watch-create','function':{'name':entry.provider_name,'arguments':json.dumps({'definition':preview['definition'],'expected_plan_id':preview['plan_id']})}}
        return daemon.AiToolTurnResult([call],'','','tool_calls',[])
    with patch.object(daemon,'_run_on_daemon_main_thread',side_effect=lambda _,callback:callback(conn)),patch.object(daemon,'_stream_ai_chat_tool_turn',side_effect=provider),patch.object(daemon,'_write_ai_chat_terminal'),patch('socket.getaddrinfo',side_effect=AssertionError('DNS')),patch('socket.socket.connect',side_effect=AssertionError('socket')):
        daemon._run_ai_chat_tool_loop('watch-consent',SimpleNamespace(last_provider_session_id=None),{'name':'synthetic','kind':'local','base_url':'http://localhost'},validated,output,active,runtime,chats)
    assert len(prompts)==1
    assert conn.execute('SELECT count(*) FROM chain_analysis_watches').fetchone()[0]==(decision=='allow_once')
    assert read_inbox(conn)['items']==[]


def test_source_expiry_is_meaningful_without_any_source_write(encrypted_book):
    from kassiber.core import chain_analysis_datasets as datasets
    from tests.test_chain_analysis_datasets import manifest, binary
    conn,_=encrypted_book
    datasets.import_dataset(conn,'profile',manifest(),binary({'subject':tid(1),'label':'Time-bounded claim','valid_until':'2026-09-08'}),format='jsonl')
    definition={'rule':'attribution_changed','query':{'mode':'trace','subject':tid(1),'chain':'bitcoin','network':'main','observer':'public'}}
    def at(instant, action):
        with patch('kassiber.core.chain_analysis.projection_store.now_iso',return_value=instant),patch.object(datasets,'now_iso',return_value=instant),patch('socket.socket.connect',side_effect=AssertionError('network')):
            return action()
    def baseline():
        plan=dispatch(conn,'ui.chain_analysis.watches.preview',definition)
        assert plan['baseline']['values']
        dispatch(conn,'ui.chain_analysis.watches.create',{'plan':plan})
        return index_revision(conn,'profile')
    before=at('2026-09-07T12:00:00Z',baseline)
    clock=conn.execute('SELECT revision FROM chain_index_clock').fetchone()[0]
    at('2026-09-08T00:00:00Z',lambda:worker_tick(conn))
    assert [item['code'] for item in read_inbox(conn)['items']]==['coverage_lost']
    after=at('2026-09-08T00:00:00Z',lambda:index_revision(conn,'profile'))
    assert after['revision']>before['revision']
    assert conn.execute('SELECT revision FROM chain_index_clock').fetchone()[0]==clock
    at('2026-09-08T00:00:01Z',lambda:worker_tick(conn))
    assert read_inbox(conn)['unread_count']==1


def add_liquid_reference(conn):
    payload=raw(1)
    payload['vout'][0]['asset']='LBTC'
    conn.execute("INSERT INTO chain_analysis_observations(profile_id,chain,network,txid,payload_json,status_json,source_name,observed_at) VALUES('profile','liquid','liquidv1',?,?,'{}','local-fixture','2026-01-01')",(tid(1),json.dumps(payload)))
    conn.commit()


def test_generic_saved_investigation_retains_cross_chain_scope(encrypted_book):
    conn,_=encrypted_book
    add_liquid_reference(conn)
    result=dispatch(conn,'ui.chain_analysis.query',{'observer':'public'})
    assert {node['chain'] for node in result['nodes']}=={'bitcoin','liquid'}
    saved=dispatch(conn,'ui.chain_analysis.cases.save',{'title':'Both chains','query':result['query'],'expected_snapshot_id':result['snapshot_id']})
    plan=dispatch(conn,'ui.chain_analysis.watches.preview',{'case_id':saved['id'],'rule':'findings_changed'})
    assert plan['definition']['query']==result['query']
    assert 'chain' not in plan['definition']['query']
    assert 'network' not in plan['definition']['query']
    assert {domain['chain'] for domain in plan['domain']['domains']}=={'bitcoin','liquid'}
    watch=dispatch(conn,'ui.chain_analysis.watches.create',{'plan':plan})
    assert watch['definition']['query']==result['query']


def test_concrete_subject_derives_bound_domain_but_bare_ambiguous_subject_cannot_start(encrypted_book):
    from kassiber.errors import AppError
    conn,_=encrypted_book
    add_liquid_reference(conn)
    plan=dispatch(conn,'ui.chain_analysis.watches.preview',{'rule':'output_spent','query':{'mode':'trace','subject':f'bitcoin:main:out:{tid(1)}:0','observer':'public'}})
    assert plan['definition']['query']['chain']=='bitcoin'
    assert plan['definition']['query']['network']=='main'
    with pytest.raises(AppError) as error:
        dispatch(conn,'ui.chain_analysis.watches.preview',{'rule':'confirmations','query':{'mode':'trace','subject':tid(1),'observer':'public'}})
    assert error.value.code=='subject_ambiguous'
