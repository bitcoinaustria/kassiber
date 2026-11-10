"""Freeze stored provenance editor reads from the unchanged Python reference."""
import argparse
import json
from pathlib import Path
import tempfile

from kassiber.db import open_db
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_reads.json'
BASE = """
UPDATE transactions SET external_id='external-in' WHERE id='in';
UPDATE transactions SET external_id='duplicate' WHERE id IN ('out','earn');
INSERT INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,amount,fiat_currency,fiat_value,acquired_at,description,review_state,created_at,updated_at) VALUES
('s-a','ws','p','fiat_purchase','Purchase Ω','BTC',123456789001,'EUR',12.34,'2024-01-01T00:00:00Z','Origin, not tax authority','reviewed','same','updated'),
('s-z','ws','p','unknown','Unknown','LBTC',NULL,NULL,NULL,NULL,NULL,'suggested','same','updated'),
('s-old','ws','p','missing_history','History','BTC',1,'',0,'','','reviewed','before','updated'),
('s-other','ws','other','income','Other book','BTC',999,'USD',1,NULL,NULL,'reviewed','z','z');
INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,explanation,uses_chain_observation,chain_data_confirmed,created_at,updated_at) VALUES
('l-a','ws','p','s-a',NULL,'in','manual_source','reviewed','strong','manual','BTC',123456789001,'BTC',123456789002,'explicit','Manual evidence Ω',0,0,'same','updated'),
('l-z','ws','p',NULL,'out','in','peg_out','suggested','exact','chain_observation','BTC',1,'LBTC',2,'unknown',NULL,1,0,'same','updated'),
('l-old','ws','p','s-z',NULL,'out','manual_source','rejected','weak','manual','LBTC',NULL,NULL,NULL,'heuristic','',0,1,'before','updated'),
('l-other','ws','other','s-other',NULL,'excluded','manual_source','reviewed','strong','manual','BTC',999,'BTC',999,'explicit',NULL,0,0,'z','z');
INSERT INTO attachments(id,workspace_id,profile_id,transaction_id,attachment_type,label,original_filename,stored_relpath,source_url,media_type,size_bytes,sha256,copied_from_attachment_id,copied_from_transaction_id,created_at) VALUES
('a-a','ws','p','in','file','  Invoice\n\t Ω  ','invoice.pdf','not-read/secret.pdf',NULL,'application/pdf',123,'hash',NULL,NULL,'same'),
('a-z','ws','p','out','url',NULL,NULL,NULL,'https://www.example.com/proof-of-origin.pdf?token=literal#reference',NULL,NULL,NULL,'a-original','original-tx','same'),
('a-old','ws','p','excluded','file',NULL,'  Original\t filename.txt  ','../never-read',NULL,NULL,0,'','',NULL,'before'),
('a-orphan','ws','p',NULL,'url','Orphan','',NULL,'https://example.com',NULL,NULL,NULL,NULL,NULL,'z'),
('a-other','ws','other','earn','url','Other book',NULL,NULL,'https://private.invalid',NULL,NULL,NULL,NULL,NULL,'z');
INSERT INTO source_funds_source_attachments(source_id,attachment_id,created_at) VALUES ('s-a','a-z','first'),('s-a','a-a','same'),('s-a','a-old','same');
INSERT INTO source_funds_link_attachments(link_id,attachment_id,created_at) VALUES ('l-a','a-z','first'),('l-a','a-old','same'),('l-a','a-a','same');
INSERT INTO source_funds_recipients(id,workspace_id,profile_id,label,kind,default_reveal_mode,notes,active,created_at,updated_at) VALUES
('r-a','ws','p','alpha','bank','minimal',NULL,1,'fixed','fixed'),
('r-z','ws','p','ALPHA','exchange','full','Exact notes Ω',1,'fixed','updated'),
('r-i','ws','p','A retired recipient','lawyer','labels_only','',0,'old','updated'),
('r-other','ws','other','Other','other','standard',NULL,1,'z','z');
INSERT INTO source_funds_cases(id,workspace_id,profile_id,target_transaction_id,target_external_id,target_amount,asset,label,reveal_mode,status,snapshot_hash,snapshot_json,recipient_id,recipient_label_snapshot,recipient_kind_snapshot,recipient_reveal_mode_snapshot,created_at,updated_at) VALUES
('c-a','ws','p','in','frozen external',123456789001,'BTC','Case Ω','minimal','ready','saved-hash','{broken','r-a','Original bank','bank','standard','same','updated'),
('c-z','ws','p','out',NULL,1,'LBTC',NULL,'full','blocked','hash','{}',NULL,NULL,NULL,NULL,'same','updated'),
('c-other','ws','other','earn','other',999,'BTC','Other','standard','ready','hash','{}',NULL,NULL,NULL,NULL,'z','z');
"""


def outcome(conn, command, args):
    from kassiber import daemon
    from kassiber.cli import handlers
    from kassiber.core import source_funds, source_funds_recipients
    try:
        if command.startswith('ui.'):
            data = daemon._ui_source_funds_payload_from_conn(conn, command, daemon._coerce_args_dict(None, args))
        else:
            args = args or {}
            hooks = daemon._source_funds_hooks()
            if command == 'source-funds.sources.list':
                data = source_funds.list_sources(conn, args.get('workspace'), args.get('profile'), hooks)
            elif command == 'source-funds.links.list':
                data = source_funds.list_links(conn, args.get('workspace'), args.get('profile'), hooks,
                                              target_transaction_ref=args.get('target_transaction'), state=args.get('state'))
            elif command == 'source-funds.cases.list':
                data = source_funds.list_cases(conn, args.get('workspace'), args.get('profile'), hooks)
            else:
                _, profile = handlers.resolve_scope(conn, args.get('workspace'), args.get('profile'))
                data = source_funds_recipients.list_recipients(conn, profile['id'])
        return dict(ok=data)
    except AppError as error:
        return dict(error=dict(code=error.code, message=str(error), hint=error.hint, details=error.details, retryable=error.retryable))
    except Exception as error:
        return dict(error=dict(code='internal_error', message=str(error), hint=None, details=None, retryable=False))


def cases():
    result = []
    def add(name, command, args=None, sql=''):
        result.append(dict(name=name, command=command, args_json=json.dumps(args), sql=BASE + sql))
    for family in ['sources', 'links', 'cases', 'recipients']:
        cli = f'source-funds.{family}.list'
        ui = f'ui.source_funds.{family}.list'
        for suffix, sql in [('populated',''), ('empty', f'DELETE FROM source_funds_{family};'),
                            ('stale', "UPDATE profiles SET journal_input_version=999;"),
                            ('source-blocked', "INSERT INTO freshness_source_states(profile_id,source_type,source_key,source_label,status,blocking_reports,updated_at) VALUES('p','wallet','cold','Cold','error',1,'fixed');"),
                            ('no-context', "DELETE FROM settings WHERE key='context_profile';")]:
            for command in [cli, ui]:
                add(command+'-'+suffix, command, {}, sql)
        add(cli+'-other-profile', cli, dict(profile='Other'))
        add(cli+'-missing-profile', cli, dict(profile='absent'))
        add(ui+'-ignores-scope', ui, dict(profile='Other', workspace='absent', unexpected=1))
        for args in [[], False, 1, 'wrong']:
            add(ui+'-bad-args-'+repr(args), ui, args)
    for target in ['in','external-in','duplicate','missing','other','  in  ','',None]:
        for command in ['source-funds.links.list','ui.source_funds.links.list']:
            add(command+'-target-'+str(target), command, dict(target_transaction=target))
    for state in ['suggested','reviewed','rejected',' Accepted ','reject','INVALID','',None,42]:
        add('state-'+str(state), 'ui.source_funds.links.list', dict(state=state))
    add('target-before-state', 'ui.source_funds.links.list', dict(target_transaction='missing', state='INVALID'))
    for active in [True,False,'yes',None]:
        add('recipients-inactive-'+str(active), 'ui.source_funds.recipients.list', dict(include_inactive=active))
    add('frozen-recipient', 'source-funds.cases.list', {}, "UPDATE source_funds_recipients SET label='Renamed',kind='other',default_reveal_mode='labels_only' WHERE id='r-a';")
    for table, column in [('source_funds_sources','amount'), ('source_funds_links','allocation_amount'), ('source_funds_links','from_allocation_amount'), ('source_funds_cases','target_amount')]:
        family = {'source_funds_sources':'sources','source_funds_links':'links','source_funds_cases':'cases'}[table]
        for n in ['-9223372036854775808','9223372036854775807','0','1','1.5','-0.5','-1.5',"'bad'", "'123x'", "' １２３ '", "''", "'-0'", "'2.0'", "'1e3'", "'nan'", "9e999"]:
            add(f'{family}-{column}-{n}',f'source-funds.{family}.list',{}, f'UPDATE {table} SET {column}={n} WHERE profile_id=\'p\';')
    urls = ['','https://docs.google.com/document/d/id','https://drive.google.com/file/d/id','https://drive.google.com/drive/folders/id','https://docs.google.com/forms/d/id','https://docs.google.com/spreadsheets/d/id','https://docs.google.com/presentation/d/id','https://docs.google.com/drawings/d/id','https://docs.google.com/unknown','https://drive.google.com/unknown',
            'https://WWW.Example.Com/a/b.pdf?q=unchanged','https://example.com/0123456789abcdef','https://example.com/a%20b%2B%C3%96.txt','https://example.com/%ff%zz','https://example.com/name.KK','https://user:password@EXAMPLE.com:443/a;b.pdf','https://[fe80::1%En0]/proof','https:///only-a-path','http:no-host.pdf','file:///never-read','https://example.com/a;params/b;c?x','https://example.com/example.com','https://[broken/path','https://[127.0.0.1]/file','https://[vxyz]/file','https://[v1.future]/file','https://x／y/file']
    urls += ['//[broken/path','//example.com/file','//[::1]/file','https://user[::1]@example.com/file','https://prefix[::1]/file','https://[::1]suffix/file','https://][::1]/file','https://[::1%]/file','https://[::1%a%b]/file','https://[v1.ab]suffix/file','https://[::1]:bad/file','http ://[broken/file','1http://[broken/file','https://example.com/a/..;p?q=literal','https://example.com/one%2Ftwo%3Bparams.pdf','https://example.com/   /proof.pdf','https://@/file','https://USER@WWW.EXAMPLE.COM/file','https://example.com:invalid/file','https://[V1.future]/file','https://[::ffff:192.0.2.1]/file']
    for index, url in enumerate(urls):
        add('url-'+str(index), 'source-funds.sources.list', {}, "UPDATE attachments SET source_url="+quote(url)+" WHERE id='a-z';")
    evidence = 'ui.source_funds.evidence.list'
    for args in [None,{},dict(limit=1),dict(limit=2),dict(limit=200),dict(limit=201),dict(limit=' ２ '),dict(limit=1.9),dict(limit=True),dict(limit=False),dict(limit=0),dict(limit=-1),dict(limit=None),dict(limit='bad'),dict(limit=[]),dict(limit=1e30),dict(limit=float('inf')),dict(limit=float('nan')),
                 dict(cursor='0'),dict(cursor='1',limit=1),dict(cursor='2',limit=1),dict(cursor='３'),dict(cursor='٢'),dict(cursor='0001'),dict(cursor='²'),dict(cursor='①'),dict(cursor='𐩀'),dict(cursor=''),dict(cursor=' 1'),dict(cursor='-1'),dict(cursor=1),dict(cursor=True),dict(cursor='2147483647'),dict(cursor='2147483648'),dict(cursor='1'*4301),dict(cursor=None),dict(cursor='bad',limit=0),dict(unknown=1,aaa=2,cursor='bad'),[],False]:
        add('evidence-'+repr(args)[:90], evidence, args)
    return result


def quote(value):
    return "'"+value.replace("'","''")+"'"


def generate():
    scratch = ROOT/'target/r5-sof/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='sof-read-oracle-', dir=scratch) as temporary:
        conn = open_db(Path(temporary)/'data')
        result = cases()
        for case in result:
            try:
                conn.executescript('SAVEPOINT sof_case;'+(ROOT/'crates/reports/tests/data/book.sql').read_text()+case['sql'])
            except Exception as error:
                raise RuntimeError(case['name']) from error
            case['expected_json'] = json.dumps(outcome(conn, case['command'], json.loads(case['args_json'])), separators=(',',':'), sort_keys=True)
            conn.execute('ROLLBACK TO sof_case');conn.execute('RELEASE sof_case')
            conn.rollback()
        conn.close()
    return json.dumps(result,ensure_ascii=True,indent=2)+'\n'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write',action='store_true')
    args=parser.parse_args()
    generated=generate()
    if args.write:
        DESTINATION.write_text(generated)
    elif DESTINATION.read_text()!=generated:
        raise SystemExit('source-funds read vectors differ; regenerate intentionally')
    print(f'{len(json.loads(generated))} stored source-funds read cases verified')


if __name__=='__main__':
    main()
