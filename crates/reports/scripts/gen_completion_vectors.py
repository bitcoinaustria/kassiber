#!/usr/bin/env python3
"""Freeze automatic saved-export completion against Python on disposable books.

Each case replays book.sql plus case SQL, publishes the named artifact files,
then registers the export exactly as a completed Kassiber export does. Tables
are dumped after Python's commit, or after the rollback a failed process makes.
"""
from pathlib import Path
import json
import sys
import tempfile
from unittest.mock import patch

import kassiber.cli.main  # noqa: F401 - the CLI's Decimal context (32 digits)
from kassiber.core import custody_filed_reports as reports
from kassiber.core.chain_observer.provenance import canonical_graph_hash, canonical_observed_quantity_hash
from kassiber.db import open_db
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'crates/reports/tests/data/completion.json'
STAMP = '2026-01-02T03:04:05Z'
TABLES = ('filed_report_snapshots', 'filed_report_chain_captures', 'filed_report_chain_dependencies')
TX = {n: f'{n:x}' * 64 for n in range(1, 16)}
BLOCK = 'AB' * 32


def q(value):
    if value is None:
        return 'NULL'
    if isinstance(value, (int, float)):
        return repr(value)
    return "'" + str(value).replace("'", "''") + "'"


def tx(ident, wallet, asset, direction, amount, raw, *, at='2024-03-01T00:00:00Z', excluded=0,
       external=None, prov=True, observed='2024-03-02T00:00:00Z', tamper=None, entry='acquisition',
       entry_at=None, entry_asset=None, quantity=None, kind=None):
    raw_json = raw if isinstance(raw, str) or raw is None else json.dumps(raw)
    row = dict(wallet_id=wallet, external_id=external, direction=direction, asset=asset,
               amount=amount, fee=0, amount_includes_fee=0)
    sql = (f"INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
           f"direction,asset,amount,fee,excluded,raw_json,kind,created_at) VALUES({q(ident)},'ws','p',{q(wallet)},"
           f"{q(external)},{q('fp-' + ident)},{q(at)},{q(direction)},{q(asset)},{amount},0,{excluded},"
           f"{q(raw_json)},{q(kind or 'deposit')},'fixed');\n")
    if prov:
        graph = canonical_graph_hash(raw_json)
        quantity_hash = canonical_observed_quantity_hash(row)
        if tamper == 'graph':
            graph = '0' * 64
        if tamper == 'quantity':
            quantity_hash = '0' * 64
        version = 2 if tamper == 'version' else 1
        sql += (f"INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,"
                f"authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,"
                f"graph_hash,quantity_hash,fee_attribution,observed_at,updated_at) VALUES({q(ident)},'ws','p',"
                f"{q(wallet)},{version},'[]','[]','bitcoin','main','r',{q(graph)},{q(quantity_hash)},'unknown',"
                f"{q(observed)},'fixed');\n")
    if entry:
        sql += (f"INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,"
                f"occurred_at,entry_type,asset,quantity,fiat_value,cost_basis,proceeds,gain_loss,created_at) VALUES("
                f"{q('je-' + ident)},'ws','p',{q(ident)},{q(wallet)},'acct',{q(entry_at or at)},{q(entry)},"
                f"{q(entry_asset or asset)},{quantity if quantity is not None else amount},0,0,0,0,'fixed');\n")
    return sql


def status(txid, confirmed=True, **extra):
    return dict(txid=txid, vin=[], vout=[], status=dict(confirmed=confirmed, block_hash=BLOCK,
                                                        block_height=800000, **extra))


WALLETS = """
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,account_id,created_at) VALUES
('chain','ws','p','Chain','descriptor','{"chain":"bitcoin","network":"main"}','acct','fixed'),
('plain','ws','p','Plain','custom','{}','acct','fixed'),
('liq','ws','p','Liquid','descriptor','{"chain":"liquid","network":"liquidv1","chain_instance_id":{"z":1,"a":"é"}}','acct','fixed');
"""
LBTC_ID = '6f0279e9ed041c3d710a9f57d0c02928416460c4b722ae3457a11eec381c526d'
CHAIN = WALLETS + ''.join([
    tx('c1', 'chain', 'BTC', 'inbound', 1000, status(TX[1]), external=TX[1]),
    tx('c2', 'chain', 'BTC', 'inbound', 2000, status(TX[2], confirmed=False)),
    tx('c3', 'chain', 'BTC', 'inbound', 3000, status(TX[3]), tamper='graph'),
    tx('c4', 'chain', 'BTC', 'inbound', 4000, status(TX[4]), prov=False),
    tx('c5', 'chain', 'BTC', 'inbound', 5000, dict(txid=TX[5], vin=[], vout=[], status={'block_height': 1})),
    tx('c6', 'chain', 'BTC', 'inbound', 6000, status(TX[6]), observed='not a time'),
    tx('c7', 'chain', 'BTC', 'inbound', 7000, {'tx': status(TX[7]) | {'status': {'confirmed': True, 'block_hash': 'xyz', 'block_height': True}}}),
    tx('c8', 'chain', 'BTC', 'inbound', 8000, status(TX[8]), excluded=1),
    tx('c9', 'liq', 'LBTC', 'outbound', 9000, dict(txid=TX[9], chain='liquid', network='liquidv1', vin=[], vout=[],
       component={'asset_id': LBTC_ID, 'asset': 'LBTC'}, status=dict(confirmed=True, block_hash=BLOCK.lower(), block_height=10**30)),
       at='2024-06-01T00:00:00Z'),
    tx('c10', 'chain', 'BTC', 'inbound', 10000, status(TX[10]), tamper='quantity'),
    tx('c11', 'chain', 'BTC', 'inbound', 11000, status(TX[11]) | {'chain_instance_id': 'lab'}, observed=' 2024-03-02 '),
    tx('c12', 'chain', 'BTC', 'inbound', 12000, status(TX[12]), at='2026-03-01T00:00:00Z'),
    tx('c13', 'chain', 'BTC', 'inbound', 13000, dict(chain='bitcoin', network='testnet', vin=[], vout=[],
       status=dict(confirmed=True)), external=TX[13]),
    tx('c14', 'chain', 'BTC', 'inbound', 14000, status(TX[14]), tamper='version'),
    tx('c15', 'chain', 'BTC', 'inbound', 15000, status(TX[15]), entry=None),
    # Graphless provider ids fall back to conflict-only physical identity.
    tx('c16', 'plain', 'BTC', 'inbound', 16000, dict(chain='bitcoin', network='testnet', vin=[], vout=[],
       status=dict(confirmed=True)), external='A' * 64),
    tx('c17', 'plain', 'LBTC', 'inbound', 17000, dict(chain='liquid', bitcoin_network='regtest', vin=[], vout=[],
       txid=' ' + 'B' * 64 + ' ', status=dict(confirmed=False, block_height=5)), external='b' * 64),
    # Generic 2024, but already 2025 in Vienna.
    tx('c21', 'chain', 'BTC', 'inbound', 21000, status('2' * 63 + 'f'), at='2024-12-31T23:30:00Z'),
    tx('c20', 'plain', 'BTC', 'inbound', 20000, dict(vin=[], vout=[], txid='D' * 64, status=dict(confirmed=True)),
       external='C' * 64),
    tx('c18', 'plain', 'LBTC', 'inbound', 18000, dict(vin=[], vout=[], status=dict(confirmed=True)), external='E' * 64),
    tx('c19', 'plain', 'BTC', 'inbound', 19000, dict(tx=dict(txid='F' * 64, vin=[], vout=[]), status=dict(confirmed=True))),
])
# The LBTC outbound c9 carries basis into the BTC receipt c1 (a reviewed rail change).
RELATION = ("INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,"
            "target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,"
            "target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,source_network,target_network,"
            "source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,occurred_at,"
            "target_occurred_at,created_at) VALUES(printf('%064d',9),'ws','p','c9','c1',printf('%064d',2),0,9000,"
            "printf('%064d',3),0,9000,'liq','chain','liquidv1','mainnet','liquid','bitcoin','LBTC','BTC',"
            "'internal_reviewed','eligible','reviewed','2024-06-01T00:00:00Z','2024-06-01T00:00:00Z','r');\n")
BINDING = ("INSERT INTO book_network_bindings(profile_id,environment_id,environment,chain_instance_id,domains_json,"
           "acknowledgements_json,inventory_digest,created_at) VALUES('p','env','main',NULL,"
           "'[{\"chain\":\"bitcoin\",\"network\":\"main\",\"domain_id\":\"bound-main\"},"
           "{\"chain\":\"liquid\",\"network\":\"liquidv1\",\"domain_id\":7}]','[]','d','fixed');\n")
JOURNAL_DIGITS = """
UPDATE journal_entries SET proceeds_exact='1234.56789012345678901234567890123',
 cost_basis_exact='0.00000000000000000000000000000001', gain_loss_exact=' 7_000.5 ' WHERE id='z';
UPDATE journal_entries SET proceeds_exact='', proceeds=1e-7, gain_loss_exact='-0' WHERE id='a';
"""


def build():
    specs = []

    def add(name, *, sql='', kind='full-report.csv', year=None, scope=None, files=None, paths=None):
        files = {'report.csv': 'complete\r\n'} if files is None else files
        specs.append(dict(name=name, sql=sql, kind=kind, year=year, scope=scope, files=files,
                          paths=list(files) if paths is None else paths))

    add('full-journal-range')
    add('full-wallet-scope', scope={'wallet_ids': ['cold']})
    add('austrian-year', kind='austrian-e1kv.csv-bundle', year=2024,
        files={'00_uebersicht.csv': 'a\r\n', '99_erläuterungen.csv': 'é\r\n', '01.csv': ''})
    add('austrian-profile', sql="UPDATE profiles SET tax_country='at';", kind='austrian-e1kv.csv-bundle', year=2025)
    add('austrian-profile-journal-range', sql="UPDATE profiles SET tax_country='at';")
    add('literal-capture-policy', sql="UPDATE profiles SET tax_country=' AT ';" + CHAIN)
    add('literal-capture-policy-year', sql="UPDATE profiles SET tax_country=' AT ';" + CHAIN,
        kind='austrian-e1kv.csv-bundle', year=2024)
    add('austrian-capture-year', sql="UPDATE profiles SET tax_country='at';" + CHAIN,
        kind='austrian-e1kv.csv-bundle', year=2024)
    add('no-journals-fallback-year', sql='DELETE FROM journal_entries;')
    add('year-zero-rejected', year=0)
    add('year-text', year='2024')
    add('digits', sql=JOURNAL_DIGITS)
    add('digits-2025', sql=JOURNAL_DIGITS, year=2025)
    add('scope-window', scope={'occurred_at_start': '2024-12-31T23:30:00Z', 'occurred_at_end': '2025-01-02T00:00:00Z'},
        kind='summary.pdf')
    add('invalid-entry-type', sql="UPDATE journal_entries SET entry_type='Bad Type' WHERE id='z';")
    add('too-many-entry-types', sql=''.join(
        f"INSERT INTO journal_entries(id,workspace_id,profile_id,transaction_id,wallet_id,account_id,occurred_at,"
        f"entry_type,asset,quantity,created_at) VALUES('many{i}','ws','p','in','cold','acct','2024-05-01T00:00:00Z',"
        f"'kind{i}','BTC',{i},'fixed');" for i in range(33)))
    add('huge-quantities', sql="UPDATE journal_entries SET quantity=-9223372036854775807 WHERE id IN ('z','a','first');")
    add('bad-exact', sql="UPDATE journal_entries SET proceeds_exact='1,5' WHERE id='z';")
    add('bad-currency', sql="UPDATE profiles SET fiat_currency='e1';")
    add('chain-dependencies', sql=CHAIN + RELATION)
    add('chain-dependencies-bound', sql=CHAIN + RELATION + BINDING)
    add('chain-wallet-scope', sql=CHAIN + RELATION, scope={'wallet_ids': ['chain']})
    add('chain-period-2024', sql=CHAIN + RELATION, kind='austrian-e1kv.csv-bundle', year=2024,
        files={'a.csv': 'a', 'b.csv': 'b'})
    add('chain-window-not-holdings', sql=CHAIN + RELATION, kind='austrian-e1kv.pdf',
        scope={'wallet_ids': ['chain'], 'occurred_at_start': '2024-04-01T00:00:00Z',
               'occurred_at_end': '2024-12-31T00:00:00Z'})
    add('chain-window-holdings', sql=CHAIN + RELATION,
        scope={'wallet_ids': ['chain'], 'occurred_at_start': '2024-04-01T00:00:00Z',
               'occurred_at_end': '2024-12-31T00:00:00Z'})
    add('chain-window-selects-period', sql=CHAIN + RELATION, kind='capital-gains.csv',
        scope={'wallet_ids': ['chain'], 'occurred_at_start': '2024-02-01T00:00:00Z',
               'occurred_at_end': '2024-12-31T00:00:00Z'}, year=2024)
    add('chain-no-relation', sql=CHAIN, scope={'wallet_ids': ['chain']})
    add('capture-rejected', sql=CHAIN + "CREATE TRIGGER reject_capture BEFORE INSERT ON filed_report_chain_captures "
        "BEGIN SELECT RAISE(ABORT,'capture_rejected'); END;")
    add('dependency-rejected', sql=CHAIN + "CREATE TRIGGER reject_dependency BEFORE INSERT ON "
        "filed_report_chain_dependencies BEGIN SELECT RAISE(ABORT,'dependency_rejected'); END;")
    add('missing-file', paths=['absent.csv'])
    add('directory-artifact', files={}, paths=['folder'])
    add('no-artifacts', files={}, paths=[])
    add('duplicate-basenames', files={'a.csv': 'x', 'sub/a.csv': 'y'})
    add('wrong-scope', scope={'wallet_ids': 'cold'})
    from gen_liquid_confirmation_vectors import raw_transactions
    for name, raw in raw_transactions().items():
        payload = dict(chain='liquid', network='liquidv1', raw_hex=raw.hex(),
                       status=dict(confirmed=True, block_hash=BLOCK, block_height=123))
        sql = WALLETS + tx('raw-hex', 'plain', 'LBTC', 'inbound', 1000, payload, external='b' * 64)
        add('liquid-raw-hex-' + name, sql=sql)
    for name, payload in (
        ('invalid', dict(raw_hex='0200')),
        ('graph-precedence', dict(raw_hex='0200', vin=[], vout=[])),
        ('txid-conflict', dict(raw_hex=raw_transactions()['explicit'].hex(), txid='a' * 64)),
    ):
        payload.update(chain='liquid', network='liquidv1', status=dict(confirmed=False))
        add('liquid-raw-hex-' + name, sql=WALLETS + tx('raw-hex', 'plain', 'LBTC', 'inbound', 1000,
            payload, external='b' * 64))
    result = []
    for spec in specs:
        with tempfile.TemporaryDirectory(prefix='kassiber-completion-oracle-') as root:
            artifacts = Path(root) / 'artifacts'
            (artifacts / 'folder').mkdir(parents=True)
            for name, content in spec['files'].items():
                (artifacts / name).parent.mkdir(parents=True, exist_ok=True)
                (artifacts / name).write_bytes(content.encode())
            conn = open_db(Path(root) / 'data')
            conn.executescript((ROOT / 'crates/reports/tests/data/book.sql').read_text() + spec['sql'])
            try:
                with patch.object(reports, 'now_iso', return_value=STAMP), \
                        patch('kassiber.core.filed_report_chain.now_iso', return_value='captured'), \
                        patch.object(reports.uuid, 'uuid4', return_value='new'):
                    payload = reports.register_saved_report_export(
                        conn, workspace_id='ws', profile_id='p', report_kind=spec['kind'],
                        artifact_paths=[artifacts / name for name in spec['paths']],
                        period_start_year=spec['year'], period_end_year=spec['year'],
                        report_scope=spec['scope'])
                expected = {'ok': payload}
            except AppError as exc:
                conn.rollback()
                expected = {'error': dict(code=exc.code, message=str(exc), hint=exc.hint, details=exc.details,
                                          retryable=exc.retryable)}
            except Exception as exc:  # noqa: BLE001 - the CLI reports these as internal errors
                conn.rollback()
                expected = {'error': dict(code='internal_error', message=str(exc), hint=None, details=None,
                                          retryable=False)}
            spec['expected'] = expected
            spec['tables'] = {table: [dict(row) for row in conn.execute(f'SELECT * FROM {table} ORDER BY 1,2,3')]
                              for table in TABLES}
            conn.close()
            result.append(spec)
    return json.dumps(result, ensure_ascii=True, indent=2) + '\n'


if __name__ == '__main__':
    content = build()
    if '--check' in sys.argv:
        assert OUT.read_text() == content, 'completion vectors drifted'
    else:
        OUT.write_text(content)
