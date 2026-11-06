"""Freeze the unchanged Python report planner on synthetic bounded graphs."""
import argparse
import json
from pathlib import Path
import random
import tempfile

from kassiber.core.source_funds_traversal import load_report_traversal
from kassiber.db import open_db
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parents[3]
DESTINATION = ROOT / 'crates/reports/tests/data/source_funds_traversal.json'


def cases():
    result = []

    def add(name, edges, *, depth=8, nodes=5000, limit=5000, target='in', profile='p'):
        result.append(dict(name=name, edges=edges, max_depth=depth, node_limit=nodes,
                           edge_limit=limit, target=target, profile=profile))

    def edge(parent, child='in', state='reviewed', created='fixed', profile='p', source=False):
        return dict(parent=parent, child=child, state=state, created=created,
                    profile=profile, source=source)

    add('empty', [])
    add('source', [edge('s', source=True)])
    add('same-id-distinct-endpoint-types', [edge('in', source=True)])
    add('parent', [edge('a'), edge('s', 'a', source=True)])
    add('branch-shared-parent', [edge('a'), edge('b'), edge('c', 'a'), edge('d', 'b'),
                               edge('c', 'd'), edge('s', 'c', source=True)])
    add('branch-shared-source', [edge('a'), edge('b'), edge('s', 'a', source=True),
                               edge('s', 'b', source=True)])
    add('duplicate-dependency', [edge('a'), edge('a'), edge('s', 'a', source=True)])
    add('suggestions-do-not-recurse', [edge('a', state='suggested'), edge('s', 'a', source=True)])
    add('rejected-do-not-recurse', [edge('a', state='rejected'), edge('s', 'a', source=True)])
    add('unknown-state-retained-only', [edge('a', state='legacy'), edge('s', 'a', source=True)])
    add('scope-p', [edge('a', profile='other'), edge('b'), edge('s', 'a', source=True)])
    add('scope-other', [edge('a', profile='other'), edge('b')], profile='other')
    add('unreachable-cycle', [edge('a', 'b'), edge('b', 'a')])
    add('self-cycle', [edge('in')])
    add('two-cycle', [edge('a'), edge('in', 'a')])
    add('three-cycle', [edge('a'), edge('b', 'a'), edge('in', 'b')])
    add('cycle-with-ready-fringe', [edge('a'), edge('b', 'a'), edge('a', 'b'),
                                  edge('z'), edge('s', 'z', source=True)])
    add('two-cycles', [edge('a'), edge('b'), edge('a', 'a'), edge('b', 'b')])
    add('sort-ready-layer', [edge('z'), edge('a'), edge('ß'), edge('Ω'), edge('𝄞'),
                             edge('b', 'a'), edge('b', 'z')])
    add('sql-order', [edge('a', state='suggested', created='a'), edge('b', created='z'),
                      edge('c', created='a'), edge('d', created='a')])
    for depth in range(5):
        add(f'depth-{depth}', [edge('a'), edge('b', 'a'), edge('c', 'b'),
                              edge('s', 'c', source=True)], depth=depth)
    for count in range(5):
        add(f'nodes-{count}', [edge('a'), edge('b'), edge('s', source=True),
                              edge('s', source=True), edge('ignored', state='suggested')], nodes=count)
        add(f'edges-{count}', [edge('a'), edge('a'), edge('s', source=True),
                              edge('ignored', state='suggested')], limit=count)
    add('existing-node-at-cap', [edge('a'), edge('a'), edge('in', 'a')], nodes=2)
    add('suggestions-at-node-cap', [edge('ignored', state='suggested'),
                                   edge('ignored-again', state='suggested')], nodes=1)
    add('empty-at-zero-edge-cap', [], limit=0)
    add('missing-target', [edge('a')], target='absent')
    add('deep-chain', [edge(f't-{n+1:02}', 'in' if n == 0 else f't-{n:02}')
                       for n in range(70)], depth=64)
    rng = random.Random(2105)
    for number in range(160):
        names = ['in', 'a', 'b', 'c', 'd', 'e', 'f', 'g']
        edges = [edge(rng.choice(names), rng.choice(names),
                      rng.choice(['reviewed'] * 5 + ['suggested', 'rejected']),
                      created=rng.choice(['a', 'b', 'same']),
                      profile=rng.choice(['p'] * 8 + ['other']),
                      source=rng.randrange(5) == 0)
                 for _ in range(rng.randrange(4, 30))]
        add(f'seeded-{number}', edges, depth=rng.randrange(1, 9),
            nodes=rng.choice([1, 3, 5, 5000]), limit=rng.choice([0, 3, 10, 5000]))
    sequence = 'WITH RECURSIVE seq(n) AS (VALUES(0) UNION ALL SELECT n+1 FROM seq WHERE n<5000) '
    for name, distinct, count in [('production-edge-cap', False, 5000),
                                   ('production-node-cap', True, 5000),
                                   ('production-exact-edge-cap', False, 4999)]:
        add(name, [])
        case = result[-1]
        source = "printf('s-%05d',n)" if distinct else "'s'"
        case['sql'] = sequence + (
            'INSERT OR IGNORE INTO source_funds_sources'
            '(id,workspace_id,profile_id,source_type,label,asset,amount,review_state,created_at,updated_at) '
            f"SELECT {source},'ws','p','income','Root','BTC',1000,'reviewed','fixed','fixed' FROM seq;")
        case['sql'] += sequence + (
            'INSERT INTO source_funds_links'
            '(id,workspace_id,profile_id,from_source_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,created_at,updated_at) '
            f"SELECT printf('link-%05d',n),'ws','p',{source},'in','manual_source','reviewed','strong','manual','BTC',1000,'BTC',1000,'explicit','fixed','fixed' FROM seq WHERE n<={count};")
        case['summary_only'] = True
    return result


def quote(value):
    return "'" + value.replace("'", "''") + "'"


def case_sql(case):
    edges = case['edges']
    txs = {(e['child'], e['profile']) for e in edges}
    txs.update((e['parent'], e['profile']) for e in edges if not e['source'])
    sources = {(e['parent'], e['profile']) for e in edges if e['source']}
    sql = []
    for tx, profile in sorted(txs):
        sql.append("INSERT OR IGNORE INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,excluded,occurred_at,created_at) VALUES(" +
                   ','.join(map(quote, [tx, 'ws', profile, 'cold', tx, 'BTC', 'inbound'])) +
                   ",1000,0,0,'2024-01-01T00:00:00Z','fixed');")
    for source, profile in sorted(sources):
        sql.append("INSERT OR IGNORE INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,amount,review_state,created_at,updated_at) VALUES(" +
                   ','.join(map(quote, [source, 'ws', profile, 'income', source, 'BTC'])) +
                   ",1000,'reviewed','fixed','fixed');")
    for index, e in enumerate(edges):
        source = quote(e['parent']) if e['source'] else 'NULL'
        parent = 'NULL' if e['source'] else quote(e['parent'])
        sql.append("INSERT INTO source_funds_links(id,workspace_id,profile_id,from_source_id,from_transaction_id,to_transaction_id,link_type,state,confidence,method,asset,allocation_amount,from_asset,from_allocation_amount,allocation_policy,created_at,updated_at) VALUES(" +
                   ','.join([quote(f'link-{index:03}'), "'ws'", quote(e['profile']), source, parent,
                             quote(e['child']), "'manual_source'", quote(e['state']), "'strong'", "'manual'",
                             "'BTC'", '1000', "'BTC'", '1000', "'explicit'", quote(e['created']), "'fixed'"]) + ');')
    return '\n'.join(sql)


def outcome(conn, case):
    result = load_report_traversal(conn, case['profile'], case['target'],
                                   max_depth=case['max_depth'], node_limit=case['node_limit'],
                                   edge_limit=case['edge_limit'])
    if case.get('summary_only'):
        retained = list(result.links.get(case['target'], ()))
        return dict(rank=dict(result.rank), truncated=result.truncated, cycle_link_id=result.cycle_link_id,
                    link_count=len(retained), first=retained[0]['id'], last=retained[-1]['id'])
    return dict(rank=dict(result.rank),
                links={key: [dict(row) for row in rows] for key, rows in result.links.items()},
                cycle_link_id=result.cycle_link_id, truncated=result.truncated)


def all_tables(conn):
    tables = [row[0] for row in conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert tuple(tables) == TABLES
    records = {table: sorted([dict(row) for row in conn.execute('SELECT * FROM "' + table + '"')],
                             key=lambda row: json.dumps(row, sort_keys=True))
               for table in tables}
    return json.dumps(records, sort_keys=True, separators=(',', ':')).encode()


def generate():
    scratch = ROOT / 'target/r5-sof-reads-2/scratch'
    scratch.mkdir(parents=True, exist_ok=True)
    result = cases()
    with tempfile.TemporaryDirectory(prefix='traversal-oracle-', dir=scratch) as temporary:
        conn = open_db(Path(temporary) / 'data')
        for case in result:
            case['sql'] = case.get('sql') or case_sql(case)
            conn.executescript('SAVEPOINT traversal_case;' +
                               (ROOT / 'crates/reports/tests/data/book.sql').read_text() + case['sql'])
            before = all_tables(conn)
            case['expected'] = outcome(conn, case)
            assert all_tables(conn) == before, case['name']
            conn.execute('ROLLBACK TO traversal_case')
            conn.execute('RELEASE traversal_case')
        conn.close()
    return json.dumps(result, ensure_ascii=True, indent=2) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    generated = generate()
    if args.write:
        DESTINATION.write_text(generated)
    elif DESTINATION.read_text() != generated:
        raise SystemExit('source-funds traversal vectors differ; regenerate intentionally')
    print(f'{len(json.loads(generated))} source-funds traversal cases verified')


if __name__ == '__main__':
    main()
