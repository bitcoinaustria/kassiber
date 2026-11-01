from __future__ import annotations

import json
from pathlib import Path
import sys
from unittest.mock import patch
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from generate_analysis_sources import assertion, cache, dataset, inventory, label, stored, TX, BLOCK
from generate_analysis_index import database
from kassiber.core.chain_analysis import projection_store as durable
from kassiber.core.chain_analysis.occurrences import OccurrenceResolver
from kassiber.core.chain_analysis.index import digest

DEST = ROOT / 'crates/wallets/tests/data/analysis_durable.json'


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), allow_nan=False)


def capture(conn):
    tables = {}
    for (name,) in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"):
        data = sorted((dict(row) for row in conn.execute(f'SELECT * FROM "{name}"')), key=canonical)
        tables[name] = digest(data)
    schema = [dict(row) for row in conn.execute("SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY name")]
    return {'tables': tables, 'table_count': len(tables), 'schema': digest(schema)}


def evaluate(case, conn=None):
    own = conn is None
    conn = database(case.get('sql', '')) if own else conn
    profile = case.get('profile', 'p')
    results = []
    try:
        for action in case['actions']:
            operation = action['operation']
            if operation == 'sql':
                conn.executescript(action['sql'])
                value = None
            elif operation == 'install':
                value = sorted(durable.install(conn))
            elif operation == 'contribute':
                table, key = action['table'], action['key']
                row = conn.execute(f'SELECT rowid AS _source_key,* FROM {table} WHERE rowid=?', (key,)).fetchone()
                value = sorted(durable._contribute(conn, profile, table, key, dict(row) if row else None, OccurrenceResolver(conn, profile)))
            elif operation == 'contributors':
                selected = durable._contributors(conn, profile, set(action['nodes']) if 'nodes' in action else None)
                value = [[table, key, row] for (table, key), row in sorted(selected.items(), key=lambda pair: (durable.PHYSICAL.index(pair[0][0]), pair[0][1]))]
            elif operation == 'synchronize':
                conn.execute('SAVEPOINT durable_fixture')
                try:
                    with patch.object(durable, 'now_iso', return_value=action.get('as_of', '2026-01-01T00:00:00Z')), patch.object(durable.uuid, 'uuid4', return_value=SimpleNamespace(hex='0123456789abcdef0123456789abcdef')), patch('kassiber.core.chain_analysis_datasets.now_iso', return_value=action.get('as_of', '2026-01-01T00:00:00Z')):
                        value = durable.synchronize(conn, profile, rebuild=action.get('rebuild', False))
                except BaseException:
                    conn.execute('ROLLBACK TO durable_fixture')
                    raise
                finally:
                    conn.execute('RELEASE durable_fixture')
            else:
                raise ValueError(operation)
            results.append({'result': value, 'state': capture(conn)})
        conn.commit()
        return results
    finally:
        if own:
            conn.close()


def vectors():
    cases = []
    def sql(text):
        return {'operation': 'sql', 'sql': text}
    def contribute(table, key=1):
        return {'operation': 'contribute', 'table': table, 'key': key}
    def add(name, actions, seed='', profile='p'):
        case = {'name': name, 'sql': seed, 'profile': profile, 'actions': actions}
        case['expected'] = evaluate(case)
        cases.append(case)
    install = {'operation': 'install'}
    add('idempotent-install', [install, install])
    state = "INSERT INTO chain_index_state VALUES('p','fixed',2,0,1,'fixed','{}',NULL);"
    add('repair-invalidates-state', [install,install], state+'DROP TABLE chain_index_nodes;')
    add('wallet-configuration', [install,contribute('transactions')], stored()+"UPDATE wallets SET config_json='{\"network\":\"regtest\",\"chain_instance_id\":\"lab\"}' WHERE id='cold';")
    add('invalid-transaction', [install,contribute('transactions')], stored(raw_json='{}', external_id='invalid'))
    triggers = database().execute("SELECT name FROM sqlite_master WHERE name LIKE 'chain_index_track_%'").fetchall()
    drop = ''.join(f'DROP TRIGGER {row[0]};' for row in triggers)
    add('install-all-triggers', [install, install], drop)
    for table in ['chain_index_sources', 'chain_index_nodes', 'chain_index_labels', 'chain_index_clock']:
        add('repair-' + table, [install, install], f'DROP TABLE {table};')
    add('transaction-replacement-and-withdrawal', [install, contribute('transactions'), sql(stored(note='changed')), contribute('transactions'), sql("UPDATE transactions SET profile_id='other' WHERE id='in';"), contribute('transactions'), {'operation':'contributors'}], stored())
    add('cache-inventory-and-readback', [install, contribute('transaction_graph_cache'), contribute('wallet_utxos'), {'operation':'contributors'}, {'operation':'contributors','nodes':[f'bitcoin:main:tx:{TX}']}, {'operation':'contributors','nodes':[]}], stored()+cache()+inventory())
    add('invalid-cache-without-nodes', [install, contribute('transaction_graph_cache'), {'operation':'contributors'}], cache(payload_json='broken'))
    add('rowid-update-and-delete', [install, sql('UPDATE transactions SET rowid=91 WHERE rowid=1;'), contribute('transactions'), contribute('transactions',91), sql("DELETE FROM transactions WHERE rowid=91;"), contribute('transactions',91)], stored())
    for active in [0,1,2]:
        add(f'assertion-{active}', [install, contribute('chain_analysis_reference_assertions'), sql('UPDATE chain_analysis_reference_assertions SET active=0;'), contribute('chain_analysis_reference_assertions')], assertion(f'{BLOCK}:0',active=active))
    add('acquired-observation', [install,contribute('chain_analysis_observations')], "INSERT INTO chain_analysis_observations(profile_id,chain,network,txid,payload_json,status_json,source_name,observed_at) VALUES('p','bitcoin','main','"+TX+"','{}','{}','local','fixed');")
    add('other-profile-and-deleted-source', [install,contribute('transactions'),contribute('transactions',999)], stored(),profile='other')
    add('overlay-and-tip-trigger-fields', [install,sql("UPDATE wallets SET label='Changed' WHERE id='cold';"),sql("UPDATE chain_analysis_acquisition_grants SET requests_used=requests_used+1;"),sql("UPDATE chain_analysis_acquisition_grants SET verified_tip_height=1;"),sql("UPDATE chain_analysis_acquisition_grants SET verified_tip_height=1;"),sql("UPDATE chain_analysis_acquisition_grants SET verified_tip_height=NULL;")], assertion(f'{BLOCK}:0'))
    return cases


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), sort_keys=True, separators=(',', ':'))+'\n')
