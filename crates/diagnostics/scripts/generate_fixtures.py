"""Regenerate synthetic Python diagnostics oracles from the repository root.

Use the locked Python 3.11 environment. Only in-memory SQLite and these fixture
files are used; no user book, HOME override, socket or provider is involved.
"""
import ast,json,pathlib
from kassiber import diagnostics as d
p=pathlib.Path('crates/diagnostics/tests/redaction.json')
cases=[('bearer','Bearer planted-secret-123'),('sk_secret','sk-planted-secret'),('recovery_assignment','recovery_phrase=legal winner thank yellow another planted seed'),('assigned_secret','password=planted-secret'),('url','https://user:planted-password@private.example/secret?token=planted'),('timestamp','2026-10-29T11:22:33Z'),('hex64','ab'*32),('xkey','upub'+'Ab12'*20),('sp_descriptor','sp(planted-sensitive-key-material)'),('sp_text','spscan1q'+'acdef'*15),('bech32','bc1q'+'acdef'*12),('path','/home/alice/private-wallet.json'),('amount','2500 msat'),('number','2500'),('ordering','auth_header=Bearer planted-token seed_phrase=legal winner thank password=secret https://a:b@c/private 2500sats')]
for token in sorted(d._SENSITIVE_KEY_TOKENS): cases.append(('key_token_'+token,token))
vectors={'text':[{'name':name,'input':v,'expected':d.sanitize_text(v)} for name,v in cases[:15]],'sensitive_keys':[],'details':[]}
for key in sorted(d._SENSITIVE_KEY_TOKENS | d._SENSITIVE_EXACT_KEYS):
 for field in [key,'nested_'+key.upper().replace('_','-')+'_suffix']:
  vectors['sensitive_keys'].append({'key':field,'expected':d._is_sensitive_key(field)})
for key in ['kind','kindness','foo','txid','addressed','count','passphrasefile','dry_run','DB_PASSWORD','sp_descriptor','data-root']:
 vectors['sensitive_keys'].append({'key':key,'expected':d._is_sensitive_key(key)})
values=[None,True,False,1,1.25,[],['secret1','secret2'],{'private_name':'planted','path':['/private/secret']},'Bearer planted-sensitive-token']
for key in ['password','wallet_config','payload','safe','balance','transaction','asset','foo']:
 for value in values:
  inp={key:value}; vectors['details'].append({'input':inp,'expected':d.sanitize_details(inp)})
# CPython IGNORECASE equivalences, word/space/digit tables and ordering.
for text in ['BEARER planted-token','ſk-planted-token','sK-planted-token','password\x1c=\x1fplanted','xé123', 'é123', '１２３ sat', 'Bearer\u2003planted-token','sp(\nplanted-secret)', '2500sat\n', 'seed=alpha beta token=abc123', 'api_key="planted"',r'C:\Users\alice\private.json','safe 42e3 mixed']:
 vectors['text'].append({'name':'unicode_'+str(len(vectors['text'])),'input':text,'expected':d.sanitize_text(text)})
p.write_text(json.dumps(vectors,indent=2,ensure_ascii=True)+'\n')
import json,sqlite3,pathlib
from kassiber.diagnostics import _state_summary, _checks_summary
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION
root=pathlib.Path('crates/diagnostics/tests')
sql='''
CREATE TABLE profiles(id TEXT, tax_country TEXT,fiat_currency TEXT,gains_algorithm TEXT,last_processed_at TEXT,last_processed_tx_count INTEGER,journal_input_version INTEGER,last_processed_input_version INTEGER,required_journal_semantics_version INTEGER,journal_semantics_version INTEGER);
INSERT INTO profiles VALUES('private-profile-id','at','EUR','MOVING_AVERAGE_AT','private-date',1,3,3,1,CURRENT_SEMANTICS),('second-id','us','USD','FIFO',NULL,0,2,1,1,0);
CREATE TABLE wallets(kind TEXT,config_json TEXT);
CREATE TABLE transactions(id TEXT,profile_id TEXT,direction TEXT,asset TEXT,excluded INTEGER,fiat_rate TEXT,fiat_value TEXT,confirmed_at TEXT,note TEXT);
INSERT INTO transactions VALUES('private-txid','private-profile-id','inbound','BTC',0,'123456','654321','private-time','private-note'),('second-txid','second-id','OUTBOUND','private-asset',1,NULL,NULL,NULL,NULL);
CREATE TABLE journal_entries(entry_type TEXT,asset TEXT,at_category TEXT);
INSERT INTO journal_entries VALUES('income','BTC','none'),('spend','private-asset','taxable');
CREATE TABLE journal_quarantines(reason TEXT);
INSERT INTO journal_quarantines VALUES('Failed https://user:secret@private.example/path amount 2500 sat');
CREATE TABLE rates_cache(pair TEXT,source TEXT);
INSERT INTO rates_cache VALUES('private-pair','private-source'),('private-pair','private-source');
CREATE TABLE attachments(attachment_type TEXT,source_url TEXT,stored_relpath TEXT);
INSERT INTO attachments VALUES('file','https://private.example/secret','/home/private-user/private.pdf'),('url','https://private.example/secret',NULL);
CREATE TABLE custody_component_transaction_memberships(profile_id TEXT,component_id TEXT);
CREATE TABLE custody_component_legs(id TEXT);
CREATE TABLE custody_component_economic_terms(term_kind TEXT,review_kind TEXT,tax_policy TEXT,source_leg_id TEXT,target_leg_id TEXT,component_id TEXT,profile_id TEXT);
INSERT INTO custody_component_transaction_memberships VALUES('private-profile-id','effective');
INSERT INTO custody_component_legs VALUES('source'),('target');
INSERT INTO custody_component_economic_terms VALUES('transaction_pair','transfer','tax-neutral','source','target','effective','private-profile-id'),('direct_swap_payout','swap','taxable','source','target','ineffective','private-profile-id');
'''
# The planted processed profile carries Python's current calculation semantics.
assert sql.count('CURRENT_SEMANTICS')==1
sql=sql.replace('CURRENT_SEMANTICS',str(JOURNAL_CALCULATION_SEMANTICS_VERSION))
# Use no effective terms for Python helper (which needs full custody leg schema);
# provide full leg columns to exercise identical effective manual review selection.
sql=sql.replace('CREATE TABLE custody_component_legs(id TEXT);','CREATE TABLE custody_component_legs(id TEXT,component_id TEXT,transaction_id TEXT,anchor_transaction_id TEXT,role TEXT,profile_id TEXT,ordinal INTEGER);').replace("INSERT INTO custody_component_legs VALUES('source'),('target');","INSERT INTO custody_component_legs VALUES('source','effective','private-txid',NULL,'source','private-profile-id',0),('target','effective','second-txid',NULL,'target','private-profile-id',1);")
sql=sql.replace('component_id TEXT,profile_id TEXT);\nINSERT INTO custody_component_transaction_memberships','component_id TEXT,profile_id TEXT,legacy_source_id TEXT,created_at TEXT,ordinal INTEGER);\nINSERT INTO custody_component_transaction_memberships').replace("'effective','private-profile-id'),('direct_swap_payout'","'effective','private-profile-id','legacy-private','private-time',0),('direct_swap_payout'").replace("'ineffective','private-profile-id');","'ineffective','private-profile-id','legacy-hidden','private-time',0);")
secret='planted-wallet-secret'
config={k:secret for k in ['descriptor','change_descriptor','addresses','source_file','sp_descriptor','backend','notes','password','token','scan_key']};config.update(chain='BITCOIN',network='MAINNET',sync_source='btcpay')
sql+='INSERT INTO wallets VALUES(\'descriptor\','+"'"+json.dumps(config)+"'),('custom','invalid-json');\n"
for table in ['workspaces','accounts','transaction_pairs','direct_swap_payouts','bip329_labels','backends']:
 sql+='CREATE TABLE '+table+'(secret TEXT);\nINSERT INTO '+table+" VALUES('planted-table-secret');\n"
backend_config={'default_backend':'private-backend','backends':{}}
urls=['https://private.example','http://user:password@127.0.0.1:1234/path?token=secret','http://192.168.1.2','http://foo.local','http://[::1]','http://localhost','http://[','no-url','', 'http://192.0.2.1','http://100.64.0.1']
for i,url in enumerate(urls):
 backend_config['backends'][f'private-{i}']={**{k:'planted-credential' for k in ['auth_header','token','username','password','cookiefile','tor_proxy']},'kind':'electrum','chain':'bitcoin','network':'mainnet','source':'database','url':url}
conn=sqlite3.connect(':memory:');conn.row_factory=sqlite3.Row;conn.executescript(sql)
(root/'state.sql').write_text(sql)
(root/'backends.json').write_text(json.dumps(backend_config,indent=2)+'\n')
(root/'state.json').write_text(json.dumps(_state_summary(conn,backend_config),indent=2)+'\n')
from argparse import Namespace
arguments = [
    {'command':'diagnostics','diagnostics_command':'collect','save':True,'format':'json','machine':True,'non_interactive':True,'debug':False,'data_root':'/private/secret','env_file':'/private/secret.env','output':'private-output','diagnostics_out':'auto','runtime_config':{'password':'secret'}},
    {'command':'rates','rates_command':'range','format':'table','machine':False,'pair':'BTC-USD','start':'not-a-date','end':'2026-10-29','project_selection_explicit':True,'limit':42,'zero':0,'text':'','empty':{},'items':['secret'], 'descriptor':'upub'+'ab'*40,'api_key':'sk-planted-secret'},
    {'command':'wallets','wallets_command':'create','dry_run':True,'amount':True,'password':False,'count':1.25,'kind':'Bearer planted-secret','file':'/private/secret','safe':'/home/private-user/secret'},
]
(root/'arguments.json').write_text(json.dumps([{'input':args,'expected':d._invocation_summary(Namespace(**args))} for args in arguments],indent=2)+'\n')
# Lone surrogates, sanitized-key collisions, and nested shapes are observable bytes.
vectors=json.loads(pathlib.Path('crates/diagnostics/tests/redaction.json').read_text())
for value in [{'password':{'1':'secret','2':'secret'}},{'1':'first','2':'second'},{'safe':'before\ud800 Bearer planted-secret after\udfff'}, {'\ud800password':'planted'},{'password_\ud800':'Bearer planted-sensitive-secret'},{'\ud800_password': {'api_key':'sk-planted-secret'}}]:
    vectors['details'].append({'input':value,'expected':d.sanitize_details(value)})
pathlib.Path('crates/diagnostics/tests/redaction.json').write_text(json.dumps(vectors,indent=2,ensure_ascii=True)+'\n')
vectors=json.loads(pathlib.Path('crates/diagnostics/tests/redaction.json').read_text())
vectors['details']=[{'input_bytes':json.dumps(v['input']), 'expected_bytes':json.dumps(v['expected'],indent=2)+'\n'} for v in vectors['details']]
pathlib.Path('crates/diagnostics/tests/redaction.json').write_text(json.dumps(vectors,indent=2,ensure_ascii=True)+'\n')
import ipaddress
localities=[]
for cls in [ipaddress._IPv4Constants,ipaddress._IPv6Constants]:
    for network in [*cls._private_networks,*cls._private_networks_exceptions]:
        for endpoint in [int(network.network_address)-1,int(network.network_address),int(network.broadcast_address),int(network.broadcast_address)+1]:
            try: address=ipaddress.ip_address(endpoint)
            except ValueError: continue
            localities.append('http://'+('['+str(address)+']' if address.version==6 else str(address)))
localities += [' http://local\nhost','\x00http://127.0.0.1','http://[::ffff:192.168.1.1]','http://[fe80::1%eth0]','http://[v1.test]', 'http://[vno.test]', 'http://[127.0.0.1]','http://host]','http://node℀private', 'http://node：123','http://foo.localhost', '//foo.local','noscheme','1http://host','http://private.example:invalid','https://user:password@localhost:8080/private?token=secret']
(root/'localities.json').write_text(json.dumps([{'url':url,'expected':d._backend_locality(url)} for url in localities],indent=2)+'\n')
# Plant credential-shaped values under every sensitive field, not just key names.
fields={key:'sk-planted-'+key.replace('_','-')+'-secret-value' for key in sorted(d._SENSITIVE_KEY_TOKENS | d._SENSITIVE_EXACT_KEYS)}
vectors=json.loads((root/'redaction.json').read_text())
for key,value in fields.items():
    candidate={key:value}
    vectors['details'].append({'input_bytes':json.dumps(candidate),'expected_bytes':json.dumps(d.sanitize_details(candidate),indent=2)+'\n'})
(root/'redaction.json').write_text(json.dumps(vectors,indent=2,ensure_ascii=True)+'\n')
arguments.append({'command':'diagnostics','diagnostics_command':'collect','format':'json',**fields})
(root/'arguments.json').write_text(json.dumps([{'input':args,'expected':d._invocation_summary(Namespace(**args))} for args in arguments],indent=2)+'\n')
